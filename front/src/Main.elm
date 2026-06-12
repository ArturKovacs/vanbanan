port module Main exposing (main)

import Browser
import Browser.Navigation as Nav
import Dict exposing (Dict)
import Element exposing (..)
import Element.Background as Background
import Element.Border as Border
import Element.Font as Font
import Element.Input as Input
import Html
import Http
import Json.Encode
import Url
import Url.Parser exposing ((</>), Parser, oneOf, s, top)


type alias SubscriptionResult = 
    { name: String
    , floor: Int
    }


-- PORTS


port subscribeToFloor : Int -> Cmd msg


port subscriptionResultHandler : (SubscriptionResult -> msg) -> Sub msg



-- MODEL


type Route
    = Home
    | FloorRoute Int


type alias Model =
    { key : Nav.Key
    , url : Url.Url
    , subscriptionStatus : Dict Int SubscriptionStatus
    , reportingBananaFoundStatus : ReportingBananaFoundStatus
    }

allFloors : List Int
allFloors = List.range 0 3

type alias Flags =
    { subscribedToFloors : List Int
    }


parseRoute : Url.Url -> Route
parseRoute url =
    case Url.Parser.parse routeParser url of
        Just route ->
            route

        Nothing ->
            Home


routeParser : Parser (Route -> a) a
routeParser =
    oneOf
        [ Url.Parser.map Home top
        , Url.Parser.map FloorRoute (s "floor" </> Url.Parser.int)
        ]


type SubscriptionStatus
    = NotSubscribed
    | Subscribing
    | Subscribed
    | SubscriptionFailed
    | NotificationsDenied
    | SubscriptionStatusUnknown String


type ReportingBananaFoundStatus
    = Idle
    | ReportingBananaFound
    | FinishedReportingBananaFound (Result Http.Error ())


init : Flags -> Url.Url -> Nav.Key -> ( Model, Cmd Msg )
init flags url key =
    let
        subscriptionStatusList =
            List.map
                (\floor ->
                    ( floor
                    , if List.member floor flags.subscribedToFloors then
                        Subscribed

                      else
                        NotSubscribed
                    )
                )
                allFloors
    in
    ( { key = key
      , url = url
      , subscriptionStatus = Dict.fromList subscriptionStatusList
      , reportingBananaFoundStatus = Idle
      }
    , Cmd.none
    )



-- UPDATE


type Floor
    = Floor Int


type Msg
    = StartSubscription Floor
    | SubscriptionResultSubscribed Floor
    | SubscriptionResultFailed Floor
    | SubscriptionResultNotificationsDenied Floor
    | SubscriptionResultUnknown Floor String
    | ReportBananaFound Floor -- Send a message to the server which will boradcase it as push messages to everyone
    | ReportBananaFoundResult (Result Http.Error ())
    | LinkClicked Browser.UrlRequest
    | UrlChanged Url.Url


subscriptionResultToMessage : SubscriptionResult -> Msg
subscriptionResultToMessage result =
    case result.name of
        "subscribed" ->
            SubscriptionResultSubscribed (Floor result.floor)

        "failed" ->
            SubscriptionResultFailed (Floor result.floor)

        "notificationsDenied" ->
            SubscriptionResultNotificationsDenied (Floor result.floor)

        other ->
            SubscriptionResultUnknown (Floor result.floor) other


update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    case msg of
        StartSubscription (Floor floor) ->
            let
                newSubscriptionStatus =
                    Dict.insert floor Subscribing model.subscriptionStatus
            in
            ( { model | subscriptionStatus = newSubscriptionStatus }, subscribeToFloor floor )

        SubscriptionResultSubscribed (Floor floor) ->
            let
                newSubscriptionStatus =
                    Dict.insert floor Subscribed model.subscriptionStatus
            in
            ( { model | subscriptionStatus = newSubscriptionStatus }, Cmd.none )

        SubscriptionResultFailed (Floor floor) ->
            let
                newSubscriptionStatus =
                    Dict.insert floor SubscriptionFailed model.subscriptionStatus
            in
            ( { model | subscriptionStatus = newSubscriptionStatus }, Cmd.none )

        SubscriptionResultNotificationsDenied (Floor floor) ->
            let
                newSubscriptionStatus =
                    Dict.insert floor NotificationsDenied model.subscriptionStatus
            in
            ( { model | subscriptionStatus = newSubscriptionStatus }, Cmd.none )

        SubscriptionResultUnknown (Floor floor) other ->
            let
                newSubscriptionStatus =
                    Dict.insert floor (SubscriptionStatusUnknown other) model.subscriptionStatus
            in
            ( { model | subscriptionStatus = newSubscriptionStatus }, Cmd.none )

        ReportBananaFound (Floor floor) ->
            ( { model | reportingBananaFoundStatus = ReportingBananaFound }
            , Http.post
                { url = "/api/message"
                , body = Http.jsonBody ( Json.Encode.object [("floor", Json.Encode.int floor )])
                , expect = Http.expectWhatever ReportBananaFoundResult
                }
            )

        ReportBananaFoundResult result ->
            ( { model | reportingBananaFoundStatus = FinishedReportingBananaFound result }, Cmd.none )

        LinkClicked urlRequest ->
            case urlRequest of
                Browser.Internal url ->
                    ( model, Nav.pushUrl model.key (Url.toString url) )

                Browser.External href ->
                    ( model, Nav.load href )

        UrlChanged url ->
            ( { model | url = url }, Cmd.none )



-- VIEW


main : Program Flags Model Msg
main =
    Browser.application
        { init = init
        , update = update
        , subscriptions = subscriptions
        , view = view
        , onUrlChange = UrlChanged
        , onUrlRequest = LinkClicked
        }


subscriptions : Model -> Sub Msg
subscriptions _ =
    subscriptionResultHandler subscriptionResultToMessage


makeSubscribeButton : Floor -> Element Msg
makeSubscribeButton floor =
    Input.button
        [ Border.rounded 10
        , Border.width 2
        , Border.color (rgb255 255 215 0)
        , paddingXY 24 14
        , centerX
        ]
        { onPress = Just (StartSubscription floor)
        , label =
            el
                []
                (text "Kérek Push Éretsítéseket")
        }


subscriptionPanel : Model -> Floor -> Element Msg
subscriptionPanel model floor =
    let
        floorInt = case floor of Floor f -> f
    in
    case Dict.get floorInt model.subscriptionStatus of
        Just NotSubscribed ->
            makeSubscribeButton floor

        Just Subscribing ->
            el
                [ Font.size 22
                , Font.color (rgb255 255 255 120)
                , centerX
                ]
                (text "Feliratkozás...")

        Just Subscribed ->
            el
                [ Font.size 22
                , Font.color (rgb255 0 255 180)
                , centerX
                ]
                (text "Feliratkoztál a push értesítésekre.")

        Just SubscriptionFailed ->
            el
                [ Font.size 22
                , Font.color (rgb255 255 100 100)
                , centerX
                ]
                (text "Nem sikerült feliratkozni a push értesítésekre.")

        Just NotificationsDenied ->
            el
                [ Font.size 22
                , Font.color (rgb255 255 100 100)
                , centerX
                ]
                (text "Értesítések megtagadva. Engedélyezd a push értesítéseket a böngésződben.")

        Just (SubscriptionStatusUnknown other) ->
            el
                [ Font.size 22
                , Font.color (rgb255 255 100 100)
                , centerX
                ]
                (text ("Ismeretlen hiba történt a push értesítések aktiválása során: " ++ other))

        Nothing ->
            el
                [ Font.size 22
                , Font.color (rgb255 255 100 100)
                , centerX
                ]
                (text ("Váratlan hiba történt. Ismeretlen emelet"))


view : Model -> Browser.Document Msg
view model =
    let
        currentRoute =
            parseRoute model.url

        content =
            case currentRoute of
                Home ->
                    homeView model

                FloorRoute floorId ->
                    floorView model (Floor floorId)
    in
    { title = "Van Banán?"
    , body = [ content ]
    }


makeFloorLink : Int -> Element Msg
makeFloorLink floorId =
    let
        floorStr =
            String.fromInt floorId
    in
    Element.link
        [ Border.rounded 10
        , Border.width 2
        , Border.color (rgb255 100 200 255)
        , paddingXY 24 14
        , centerX
        ]
        { url = "/floor/" ++ floorStr
        , label =
            el
                []
                (text (floorStr ++ ". Emelet"))
        }


homeView : Model -> Html.Html Msg
homeView model =
    layout
        [ Background.color (rgb255 35 35 35)
        , Font.color (rgb255 255 255 120)
        ]
    <|
        column
            [ width fill
            , height fill
            , spacing 24
            , centerX
            , centerY
            , padding 24
            ]
            (el
                [ Font.size 36
                , Font.bold
                , centerX
                ]
                (text "Van Banán?")
                :: List.map makeFloorLink allFloors
            )


floorView : Model -> Floor -> Html.Html Msg
floorView model floor =
    let
        floorStr =
            case floor of
                Floor n ->
                    String.fromInt n
    in
    layout
        [ Background.color (rgb255 35 35 35)
        , Font.color (rgb255 255 255 120)
        ]
    <|
        column
            [ width fill
            , height fill
            , spacing 24
            , centerX
            , centerY
            , padding 24
            ]
            [ el
                [ Font.size 36
                , Font.bold
                , centerX
                ]
                (text (floorStr ++ ". Emelet"))
            , subscriptionPanel model floor
            , Input.button
                [ Border.rounded 10
                , Border.width 2
                , Border.color (rgb255 255 215 0)
                , paddingXY 24 14
                , centerX
                ]
                { onPress = Just (ReportBananaFound floor)
                , label =
                    el
                        []
                        (text "Látok banánt a konyhában!")
                }
            , Element.link
                [ Border.rounded 10
                , Border.width 2
                , Border.color (rgb255 100 200 255)
                , paddingXY 24 14
                , centerX
                ]
                { url = "/"
                , label =
                    el
                        []
                        (text "Emeletek")
                }
            ]

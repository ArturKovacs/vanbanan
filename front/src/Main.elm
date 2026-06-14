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
    { name : String
    , floor : Int
    }


type alias UnsubscribeResult =
    { name : String
    , floor : Int
    }



-- PORTS


portResultOkName : String
portResultOkName =
    "ok"


portResultFailedName : String
portResultFailedName =
    "failed"


port subscribeToFloor : Int -> Cmd msg


port subscriptionResultHandler : (SubscriptionResult -> msg) -> Sub msg


port unsubscribeFromFloor : Int -> Cmd msg


port unsubscribeResultHandler : (UnsubscribeResult -> msg) -> Sub msg



-- MODEL


type Route
    = Home
    | FloorRoute Int


type alias Model =
    { key : Nav.Key
    , url : Url.Url
    , subscriptionStatuses : Dict Int SubscriptionStatus
    , bananaFoundStatuses : Dict Int BananaFoundStatus
    }


allFloors : List Int
allFloors =
    List.range 0 3


type alias Flags =
    { subscribedToFloors : List Int
    , bananaStates : List ( Int, Bool )
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
    | Unsubscribing
    | UnsubscribeFailed


type BananaFoundStatus
    = BananaNotFound
    | ReportingBananaFound
    | BananaFound
    | ReportingBananaNotFound



-- | FinishedReportingBananaFound (Result Http.Error ())


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

        bananaStatuses =
            Dict.fromList
                (List.map
                    (\( floor, hasBanana ) ->
                        ( floor
                        , if hasBanana then
                            BananaFound

                          else
                            BananaNotFound
                        )
                    )
                    flags.bananaStates
                )
    in
    ( { key = key
      , url = url
      , subscriptionStatuses = Dict.fromList subscriptionStatusList
      , bananaFoundStatuses = bananaStatuses
      }
    , Cmd.none
    )



-- UPDATE


type Floor
    = Floor Int


floorToInt : Floor -> Int
floorToInt floor =
    case floor of
        Floor f ->
            f


type Msg
    = StartSubscription Floor
    | GotSubscribeOk Floor
    | GotSubscribeError Floor
    | StartRemovingSubscription Floor
    | GotUnsubscribeOk Floor
    | GotUnsubscribeError Floor
    | ReportBananaFound Floor -- Send a message to the server which will boradcase it as push messages to everyone
    | ReportBananaFoundResult Floor (Result Http.Error ())
    | ReportBananaNotFound Floor
    | ReportBananaNotFoundResult Floor (Result Http.Error ())
    | LinkClicked Browser.UrlRequest
    | UrlChanged Url.Url


subscriptionResultToMessage : SubscriptionResult -> Msg
subscriptionResultToMessage result =
    if result.name == portResultOkName then
        GotSubscribeOk (Floor result.floor)

    else if result.name == portResultFailedName then
        GotSubscribeError (Floor result.floor)

    else
        let
            _ =
                Debug.log "Received unexpected result" result.name
        in
        GotSubscribeError (Floor result.floor)


unsubscribeResultToMessage : UnsubscribeResult -> Msg
unsubscribeResultToMessage result =
    if result.name == portResultOkName then
        GotUnsubscribeOk (Floor result.floor)

    else if result.name == portResultFailedName then
        GotUnsubscribeError (Floor result.floor)

    else
        let
            _ =
                Debug.log "Received unexpected result" result.name
        in
        GotUnsubscribeError (Floor result.floor)


update : Msg -> Model -> ( Model, Cmd Msg )
update msg model =
    let
        changeSubscription : Floor -> SubscriptionStatus -> Model
        changeSubscription floor newSubscriptionStatus =
            let
                floorInt =
                    case floor of
                        Floor f ->
                            f

                newSubscriptionStatuses =
                    Dict.insert floorInt newSubscriptionStatus model.subscriptionStatuses
            in
            { model | subscriptionStatuses = newSubscriptionStatuses }

        changeBananaStatus : Floor -> BananaFoundStatus -> Model
        changeBananaStatus floor newBananaStatus =
            let
                floorInt =
                    case floor of
                        Floor f ->
                            f

                newBananaStatuses =
                    Dict.insert floorInt newBananaStatus model.bananaFoundStatuses
            in
            { model | bananaFoundStatuses = newBananaStatuses }
    in
    case msg of
        StartSubscription floor ->
            ( changeSubscription floor Subscribing
            , subscribeToFloor
                (case floor of
                    Floor f ->
                        f
                )
            )

        GotSubscribeOk floor ->
            ( changeSubscription floor Subscribed, Cmd.none )

        GotSubscribeError floor ->
            ( changeSubscription floor SubscriptionFailed, Cmd.none )

        StartRemovingSubscription floor ->
            ( changeSubscription floor Unsubscribing
            , unsubscribeFromFloor
                (case floor of
                    Floor f ->
                        f
                )
            )

        GotUnsubscribeOk floor ->
            ( changeSubscription floor NotSubscribed, Cmd.none )

        GotUnsubscribeError floor ->
            ( changeSubscription floor UnsubscribeFailed, Cmd.none )

        ReportBananaFound floor ->
            let
                floorInt =
                    case floor of
                        Floor f ->
                            f
            in
            ( changeBananaStatus floor ReportingBananaFound
            , Http.post
                { url = "/api/banana"
                , body = Http.jsonBody (Json.Encode.object [ ( "floor", Json.Encode.int floorInt ), ( "has_banana", Json.Encode.bool True ) ])
                , expect = Http.expectWhatever (ReportBananaFoundResult floor)
                }
            )

        ReportBananaFoundResult floor result ->
            let
                newBananaFoundStatus =
                    case result of
                        Ok _ ->
                            BananaFound

                        Err _ ->
                            BananaNotFound
            in
            ( changeBananaStatus floor newBananaFoundStatus, Cmd.none )

        ReportBananaNotFound floor ->
            let
                floorInt =
                    case floor of
                        Floor f ->
                            f
            in
            ( changeBananaStatus floor ReportingBananaNotFound
            , Http.post
                { url = "/api/banana"
                , body = Http.jsonBody (Json.Encode.object [ ( "floor", Json.Encode.int floorInt ), ( "has_banana", Json.Encode.bool False ) ])
                , expect = Http.expectWhatever (ReportBananaNotFoundResult floor)
                }
            )

        ReportBananaNotFoundResult floor result ->
            let
                newBananaFoundStatus =
                    case result of
                        Ok _ ->
                            BananaNotFound

                        Err _ ->
                            BananaFound
            in
            ( changeBananaStatus floor newBananaFoundStatus, Cmd.none )

        LinkClicked urlRequest ->
            case urlRequest of
                Browser.Internal url ->
                    ( model, Nav.pushUrl model.key (Url.toString url) )

                Browser.External href ->
                    ( model, Nav.load href )

        UrlChanged url ->
            ( { model | url = url }, Cmd.none )



-- VIEW

myYellow : Color
myYellow =
    rgb255 255 255 120


myBlue : Color
myBlue =
    rgb255 100 200 255


myGray : Color
myGray =
    rgb255 35 35 35

myWhite : Color
myWhite =
    rgb255 210 210 210

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
    Sub.batch
        [ subscriptionResultHandler subscriptionResultToMessage
        , unsubscribeResultHandler unsubscribeResultToMessage
        ]

makeBananaStatusPanel : Model -> Floor -> Element Msg
makeBananaStatusPanel model floor =
    let
        floorInt = floorToInt floor
        hasBanana = case Dict.get floorInt model.bananaFoundStatuses of
            Just BananaFound -> True
            _ -> False

        bgColor = if hasBanana then myYellow else myGray
        fontColor = if hasBanana then myGray else myWhite
        innerText = if hasBanana then "Van Banán" else "Nincs Banán :("
    in
    el
        [ Background.color bgColor
        , Font.color fontColor
        , Border.rounded 10
        , Font.size 32
        , Font.bold
        , padding 32
        , centerX
        ]
        (text innerText)

makeSubscriptionPanel : Model -> Floor -> Element Msg
makeSubscriptionPanel model floor =
    let
        floorInt =
            case floor of
                Floor f ->
                    f

        ( isSubscribed, inProgress ) =
            case Dict.get floorInt model.subscriptionStatuses of
                Just Subscribed ->
                    ( True, False )

                Just Subscribing ->
                    ( False, True )

                Just Unsubscribing ->
                    ( True, True )

                _ ->
                    ( False, False )
    in
    Element.row [ centerX, spacing 8 ]
        [ Input.checkbox []
            { onChange =
                \shouldSubscribe ->
                    if shouldSubscribe then
                        StartSubscription floor

                    else
                        StartRemovingSubscription floor
            , icon = Input.defaultCheckbox
            , checked = isSubscribed
            , label =
                Input.labelLeft [ padding 5 ]
                    (text "Kérek Push Értesítéseket")
            }
        , el [ width (px 20) ]
            (if inProgress then
                text "⏳"

             else
                text ""
            )
        ]


makeFloorLink : Model -> Int -> Element Msg
makeFloorLink model floorInt =
    let
        floorStr =
            String.fromInt floorInt
        hasBanana = case Dict.get floorInt model.bananaFoundStatuses of
            Just BananaFound -> True
            _ -> False
    in
    row [ centerX
        , width (px 200)
        ]
        [
            if hasBanana then el [] (text "🍌") else Element.none
            , Element.link
                [ Border.rounded 10
                , Border.width 2
                , Border.color myBlue
                , paddingXY 20 14
                , centerX
                ]
                { url = "/floor/" ++ floorStr
                , label =
                    el
                        [ width fill
                        , Font.center
                        ]
                        (text (floorStr ++ ". Emelet"))
                }
            , if hasBanana then el [] (text "🍌") else Element.none
        ]


makeBananaReportButton : Model -> Floor -> Element Msg
makeBananaReportButton model floor =
    let
        floorInt =
            floorToInt floor

        notFoundAttributes = [ Border.color myYellow ]
        foundAttributes = [ Border.color myWhite, Border.dotted]

        reportBananaTuple =
            ( "Módosítom, van banán a konyhában!"
            , Just (ReportBananaFound floor)
            , notFoundAttributes
            )

        ( innerText, onPress, attributes ) =
            case Dict.get floorInt model.bananaFoundStatuses of
                Just BananaNotFound ->
                    reportBananaTuple

                Just BananaFound ->
                    ( "Módosítom, elfogyott a banán :(", Just (ReportBananaNotFound floor), foundAttributes )

                Just ReportingBananaFound ->
                    ( "⏳", Nothing, notFoundAttributes )

                Just ReportingBananaNotFound ->
                    ( "⏳", Nothing, foundAttributes )

                {-
                   Initially the banana status database is empty,
                   so no floor will be found in the dictionary.
                   This is normal, and it means that no banana is found for the floors.
                -}
                Nothing ->
                    reportBananaTuple
    in
    Input.button
        ([ Border.rounded 10
        , Border.width 2
        , paddingXY 24 14
        , centerX
        ] ++
        attributes)
        { onPress = onPress
        , label =
            el
                []
                (text innerText)
        }


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


homeView : Model -> Html.Html Msg
homeView model =
    layout
        [ Background.color myGray
        , Font.color myYellow
        ]
    <|
        column
            [ width fill
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
                :: List.map (makeFloorLink model) allFloors
            )


floorView : Model -> Floor -> Html.Html Msg
floorView model floor =
    let
        floorStr =
            String.fromInt (floorToInt floor)
    in
    layout
        [ Background.color myGray
        , Font.color myYellow
        ]
    <|
        column
            [ width fill
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
            , makeBananaStatusPanel model floor
            , makeSubscriptionPanel model floor
            , makeBananaReportButton model floor
            , Element.link
                [ Border.rounded 10
                , Border.width 2
                , Border.color myBlue
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
